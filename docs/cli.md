# CLI command reference

Reference for **Kegari `0.2.0-alpha.10`**, using the `kegari-v0` executable.
This development preview supports CLI inspection; end-to-end operation remains
under validation. Operational examples below describe command usage, not a
recommendation to activate the preview.

## Invocation

```text
kegari-v0 [--target NAME] [--json] [COMMAND]
```

Omitting `COMMAND` runs `overview`. It does not start a service or orchestration.
Before local setup, commands that need a runtime report `target_unavailable`.
Help and version inspection work without a registered runtime:

```sh
kegari-v0 --help
kegari-v0 --version
kegari-v0 setup --help
kegari-v0 help workspace configure
```

Use `-h` or `--help` on a command for its grammar; use `-V` or `--version` at the
root for the application version and build identity. If using a source build,
replace `kegari-v0` with `./target/release/kegari-v0`.

### Global options

| Option | Meaning | Default |
| --- | --- | --- |
| `--target NAME` | Select a named remote HTTPS runtime saved on this client. | Registered local runtime over its Unix socket. |
| `--json` | Emit a complete JSON response envelope. | Human-readable output. |

These options can accompany the commands below. `target` and `service` commands
reject `--target`: they manage the invoking machine. A missing or unreachable
selected runtime never falls back to another target.

### Notation and shared inputs

Synopses use `UPPERCASE` for values, `[brackets]` for optional arguments and `|`
for alternatives. Do not type the brackets. Examples use fictional project names
and shell variables; replace them with your actual values before use.

| Value | Meaning |
| --- | --- |
| `PARENT` | A unique parent issue key such as `APP-123`, or a qualified reference such as `product/APP-123`. |
| `WORK` | A parent or child work reference, as accepted by the command. |
| `WORKSPACE` | A workspace reference resolved by the runtime; use its UUID when a name is ambiguous. |
| `NAME` | A connection, workspace or saved target name. |
| `UUID` | A non-nil UUID; examples use `$OPERATION_UUID`, `$RUNTIME_UUID`, `$WORKSPACE_UUID` and `$REQUEST_UUID`. |
| `TOKEN` | An exact token returned by the runtime; examples use `$RESULT_TOKEN` or `$PLAN_TOKEN`. |
| `REVISION` | The observed configuration revision, represented by `$CONFIG_REVISION` in examples. |
| `GENERATION` | The observed integer control generation, represented by `$CONTROL_GENERATION`. |
| `PATH` | A path on the machine running the CLI, unless the command says otherwise. |

The runtime resolves work references and refuses ambiguity before applying a
change. `--workspace` is a command-specific option, not a global option.

Commands that accept `--request-id UUID` generate one request UUID if it is
omitted. Preserve that identity when an outcome is uncertain; see
[requests and recovery](#requests-and-recovery). It is not a global flag.

## Command index

| Section | Commands |
| --- | --- |
| [Inspection](#inspection) | [overview](#overview), [status](#status), [show](#show), [open](#open) |
| [Review and delivery](#review-and-delivery) | [approve](#approve), [correct](#correct), [release](#release) |
| [Configuration](#configuration) | [setup](#setup), [connection list](#connection-list), [connection add](#connection-add), [connection reconnect](#connection-reconnect), [workspace list](#workspace-list), [workspace add](#workspace-add), [workspace configure](#workspace-configure), [config show](#config-show), [config check](#config-check) |
| [Orchestration](#orchestration) | [start](#start), [pause](#pause), [resume](#resume), [stop](#stop) |
| [Recovery and cleanup](#recovery-and-cleanup) | [operation show](#operation-show), [operation reconcile](#operation-reconcile), [retry](#retry), [release recover](#release-recover), [cleanup](#cleanup) |
| [Evaluation](#evaluation) | [evaluation run](#evaluation-run) |
| [Remote targets](#remote-targets) | [target list](#target-list), [target add](#target-add) |
| [Local service](#local-service) | [service install](#service-install), [service start](#service-start), [service stop](#service-stop) |

## Inspection

### overview

**Read-only.** Show work grouped into Ready for release, Ready for review,
Blocked and Running, with supplementary history and runtime health.

```text
kegari-v0 overview [--workspace WORKSPACE]
```

`--workspace` restricts the view to one workspace. Omit it to inspect the
runtime's overall view.

```sh
kegari-v0 --target studio overview --workspace product
```

### status

**Read-only.** Inspect runtime state, control generation, health and blockers.
This is the place to check whether a requested lifecycle change has taken effect.

```text
kegari-v0 status
```

There are no command-specific options.

```sh
kegari-v0 --json status
```

### show

**Read-only.** Show a parent's current details or retained history, including
evidence, attempts and completion or blocker information.

```text
kegari-v0 show PARENT [--workspace WORKSPACE]
```

`PARENT` is required. `--workspace` disambiguates or restricts its resolution.
Retained history does not make old work eligible for new mutations.

```sh
kegari-v0 show product/APP-123
```

### open

**Reads runtime state; opens a browser on this client by default.** Obtain the
parent's authoritative Linear URL.

```text
kegari-v0 open PARENT [--workspace WORKSPACE] [--url-only]
```

`PARENT` is required; `--workspace` restricts resolution. `--url-only` returns
the URL without launching a browser. Browser opening accepts only an
authoritative HTTPS `linear.app` URL and never runs on the remote host.

```sh
kegari-v0 open product/APP-123 --url-only
```

## Review and delivery

These commands change workflow state and require the corresponding operator
permissions. Result tokens identify the precise evidence or accepted result
being acted on; a stale token is refused.

### approve

**Changes state.** Record human acceptance of the specified parent result.
Acceptance and release are separate actions.

```text
kegari-v0 approve PARENT --result TOKEN [--request-id UUID]
```

Both `PARENT` and `--result` are required. Use the exact evidence token for the
result you reviewed.

```sh
kegari-v0 approve product/APP-123 --result "$RESULT_TOKEN"
```

### correct

**Changes state.** Submit a correction to a parent.

```text
kegari-v0 correct PARENT (--message TEXT | --message-file PATH | --stdin) \
  [--request-id UUID]
```

Choose exactly one input source. Corrections must be nonempty UTF-8 without NUL
bytes, up to 1 MiB. `--message-file` reads a file on the client machine;
`--stdin` reads standard input. No editor opens automatically.

```sh
kegari-v0 correct product/APP-123 --message-file ./correction.txt
```

### release

**Changes state and may initiate delivery.** Request release of the exact
accepted parent result, subject to current delivery gates.

```text
kegari-v0 release PARENT --result TOKEN [--request-id UUID]
```

`PARENT` and `--result` are required, even though the combined release/recovery
help grammar presents them as optional. Use the exact accepted result token.
For an interrupted release, see [release recover](#release-recover).

```sh
kegari-v0 release product/APP-123 --result "$RESULT_TOKEN"
```

## Configuration

Configuration files are typed TOML; see the
[configuration guide](../crates/kegari-v0/docs/configuration.md) for their shape.
Commands using `--from PATH` read files on the client, including for a remote
runtime. `--from -` reads configuration from stdin. Inputs are limited to 1 MiB
and must be nonempty UTF-8 without NUL bytes.

Where guidance is supported, omitting `--from` requires an interactive terminal.
`--noninteractive` requires complete supplied input and never fills in missing
values. The initial local setup guide asks individual questions and requires
`create` after review. Other guided configuration accepts a whole TOML document
terminated by a line containing only `.`; EOF before that terminator cancels.

### setup

**Changes state.** Initialize a previously unregistered local runtime, or submit
configuration to the selected existing runtime.

```text
kegari-v0 setup [--from PATH] [--noninteractive] [--request-id UUID]
```

`--from` supplies a complete configuration; otherwise interactive guidance is
used. Initial local setup records stopped orchestration intent and does not
install or start the service. Explicit remote setup never falls back to creating
a local runtime. Partial or stale registration requires reconciliation rather
than replacement setup.

```sh
kegari-v0 setup --from ./runtime.toml --noninteractive
```

### connection list

**Read-only.** List connections configured on the selected runtime.

```text
kegari-v0 connection list
```

There are no command-specific options.

```sh
kegari-v0 connection list
```

### connection add

**Changes state and verifies provider identity.** Add a named connection.

```text
kegari-v0 connection add NAME --kind KIND [--from PATH] [--secret-stdin] \
  [--noninteractive] [--request-id UUID]
```

| Argument or option | Meaning |
| --- | --- |
| `NAME` | Required connection name. |
| `--kind KIND` | Required: `linear`, `github` or `codex`; must match the TOML connection kind. |
| `--from PATH` | Connection TOML; omission uses interactive guidance. |
| `--secret-stdin` | Read an optional credential separately from non-terminal stdin. |
| `--noninteractive` | Require supplied configuration instead of prompting. |

`--secret-stdin` imports Linear/GitHub credentials; Codex instead requires
authentication through its configured isolated account home. Supply `--from PATH`
with secret input: guided configuration needs terminal stdin, while secret input
must be non-terminal. `--secret-stdin` cannot be combined with `--from -`.
Secret input is limited to 64 KiB; one final LF and its optional preceding CR are
removed. It must be valid UTF-8 and is excluded from ordinary command output.
The runtime verifies the supported provider identity before importing the
credential. A credential reference on the client is not automatically a
credential on a remote runtime.

```sh
kegari-v0 connection add github --kind github --from ./github.toml --noninteractive
```

The example uses the credential reference declared in the configuration. Use
`--secret-stdin` with a protected input source when importing a new credential.

### connection reconnect

**Changes connection verification state.** Recheck an existing connection using
its configured credential or Codex authentication home.

```text
kegari-v0 connection reconnect NAME [--noninteractive] [--request-id UUID]
```

`NAME` is required. `--noninteractive` selects noninteractive command handling;
this command does not provide a login UI or accept a replacement secret. Complete
any required account authentication through the supported provider flow first.

```sh
kegari-v0 connection reconnect codex --noninteractive
```

### workspace list

**Read-only.** List workspaces configured on the selected runtime.

```text
kegari-v0 workspace list
```

There are no command-specific options.

```sh
kegari-v0 workspace list
```

### workspace add

**Changes state.** Add a named workspace with its connections, scope and policy.

```text
kegari-v0 workspace add NAME [--from PATH] [--noninteractive] [--request-id UUID]
```

`NAME` is required. `--from` supplies workspace TOML, or omission opens
interactive guidance. `--noninteractive` requires supplied input. The runtime
validates the workspace against the complete configuration before applying it.

```sh
kegari-v0 workspace add product --from ./workspace.toml --noninteractive
```

### workspace configure

**Changes state.** Apply a workspace configuration against an observed revision.

```text
kegari-v0 workspace configure ID --from PATH --expected-config REVISION \
  [--noninteractive] [--request-id UUID]
```

`ID`, `--from` and `--expected-config` are required. Use the workspace's identity
and the exact current configuration revision; stale revisions are refused.
`--noninteractive` is accepted, but this form always requires a supplied file
or explicit `--from -` input.

```sh
kegari-v0 workspace configure "$WORKSPACE_UUID" --from ./workspace.toml \
  --expected-config "$CONFIG_REVISION" --noninteractive
```

### config show

**Read-only.** Show the selected runtime's configuration with secret values
excluded.

```text
kegari-v0 config show [--workspace WORKSPACE] [--noninteractive]
```

`--workspace` limits the view; omission shows the runtime configuration.
`--noninteractive` is accepted but this read does not prompt.

```sh
kegari-v0 --json config show --workspace product
```

### config check

**Checks configuration without applying changes.** Validate configuration and
report prerequisites, connection checks and blockers.

```text
kegari-v0 config check [--workspace WORKSPACE] [--noninteractive]
```

`--workspace` restricts the check; omission checks the configured scope.
`--noninteractive` is accepted but this command does not prompt. Checks can
contact providers and run a bounded Codex metadata probe; this is not an offline
or process-free inspection. A successful command can report a blocked
configuration. Read the report: it does not establish operational qualification.

```sh
kegari-v0 config check --workspace product --noninteractive
```

## Orchestration

These commands change intent for the **whole runtime** and do not accept a
workspace filter. They are distinct from the local [service commands](#local-service).

| Shared option | Meaning |
| --- | --- |
| `--expected-control GENERATION` | Require an exact observed control generation. If omitted, the client reads it once before submitting; stale conflicts are not automatically retried. |
| `--request-id UUID` | Supply the request identity instead of generating one. |
| `--wait` | On `start` and `stop` only, observe the accepted operation until completion or the waiting deadline. Default: do not wait. |
| `--timeout SECONDS` | Requires `--wait`; range 1–86400 seconds, default 30. Bounds observation, not the operation itself. |

If waiting expires, the response retains the operation identity and can report
`wait_timeout: true` and `operation_continues: true`. This can still exit with code
`0`. It does not force, stop or repeat the operation; inspect its state separately.

### start

**Changes state.** Request orchestration startup through the running control
service, subject to recovery and readiness checks.

```text
kegari-v0 start [--expected-control GENERATION] [--wait] [--timeout SECONDS] \
  [--request-id UUID]
```

Use `resume` if the runtime is paused: `start` preserves pause and reports
`requires_resume`. It does not install or launch the control service.

```sh
kegari-v0 start --wait --timeout 30
```

### pause

**Changes state.** Stop admitting new work and let active work drain while
keeping orchestration available.

```text
kegari-v0 pause [--expected-control GENERATION] [--request-id UUID]
```

This command has no waiting option. Inspect `status` for the resulting state.

```sh
kegari-v0 pause --expected-control "$CONTROL_GENERATION"
```

### resume

**Changes state.** Resume admission after a pause, subject to current gates.

```text
kegari-v0 resume [--expected-control GENERATION] [--request-id UUID]
```

It does not start a stopped orchestration process; a stopped runtime reports
`requires_start`. There is no waiting option.

```sh
kegari-v0 resume
```

### stop

**Changes state.** Stop admission, drain work and exit orchestration. The control
service remains available for inspection and a later `start`.

```text
kegari-v0 stop [--force] [--expected-control GENERATION] \
  [--wait] [--timeout SECONDS] [--request-id UUID]
```

Without `--force`, shutdown drains work normally. `--force` requests bounded
cancellation of owned work and preserves working files; it does not waive
ownership or recovery checks. Shared waiting options are described above.

```sh
kegari-v0 stop --wait --timeout 30
```

## Recovery and cleanup

### Requests and recovery

A mutation is submitted once with its request UUID. A missing acknowledgement
does not prove failure or justify sending a new mutation. Keep the returned
`request_id` and any `operation_id`, inspect status and operation evidence, and
follow the reported recovery action. A request ID and an operation ID are
different identities; do not substitute one for the other.

### operation show

**Read-only.** Inspect a known operation's recorded state and outcome.

```text
kegari-v0 operation show UUID
```

The operation UUID is required. There are no command-specific options.

```sh
kegari-v0 operation show "$OPERATION_UUID"
```

### operation reconcile

**Changes state.** Request reconciliation of a known operation using retained
records and observed outcomes.

```text
kegari-v0 operation reconcile UUID [--request-id UUID]
```

The positional UUID identifies the operation being reconciled. `--request-id`
identifies this reconciliation request. Reconciliation can remain blocked or
uncertain; it is not a blanket instruction to repeat the original effect.

```sh
kegari-v0 operation reconcile "$OPERATION_UUID" --request-id "$REQUEST_UUID"
```

### retry

**Changes state.** Request a new bounded attempt for eligible work after its
blocker and prior ownership have been reconciled.

```text
kegari-v0 retry WORK --reason TEXT [--request-id UUID]
```

Both the work reference and a nonempty reason of at most 4096 bytes are required.
This grants an eligible new phase budget; it is not a transport retry or a way to
bypass an uncertain previous operation.

```sh
kegari-v0 retry product/APP-124 \
  --reason 'Dependency restored and prior attempt reconciled'
```

### release recover

**Changes state and may permit further delivery.** Request an explicit recovery
decision for an existing parent release operation.

```text
kegari-v0 release recover PARENT --operation UUID --action ACTION --reason TEXT \
  [--result TOKEN] [--request-id UUID]
```

| Argument or option | Meaning |
| --- | --- |
| `PARENT` | Required parent work reference. |
| `--operation UUID` | Required release operation to recover. |
| `--action ACTION` | Required: `resume`, `abandon`, `resolve` or `supersede`. |
| `--reason TEXT` | Required nonempty explanation of the recovery decision. |
| `--result TOKEN` | Required for `supersede`: the current corrected result token. |

`resume` continues an unfinished plan for the same result after outcomes are
known. `abandon` retires known unstarted work; it does not cancel or roll back
provider effects. `resolve` clears a known coherent failure hold after fresh
observation, without recording success. `supersede` replaces a held predecessor
with a corrected, accepted result after its effects are known and local work has
retired. The runtime checks each decision against release state, evidence and
permissions; unresolved uncertainty still blocks recovery.

Place recovery options after `recover`; do not mix them with parent, result or
request options on the outer `release` command.

```sh
kegari-v0 release recover product/APP-123 --operation "$OPERATION_UUID" \
  --action resume --reason 'Provider outcome reconciled'
```

### cleanup

**Read-only by default; changes state with `--apply`.** Inspect cleanup eligibility
and the exact plan for one operation, then explicitly apply that plan if eligible.

```text
kegari-v0 cleanup --operation UUID [--apply --plan TOKEN] [--request-id UUID]
```

`--operation` is required. `--apply` and `--plan` must be supplied together; use
the unchanged token from the inspected plan. Applying can remove eligible local
resources and remains subject to ownership and recovery checks.

```sh
kegari-v0 cleanup --operation "$OPERATION_UUID"
```

After inspecting the returned plan:

```sh
kegari-v0 cleanup --operation "$OPERATION_UUID" --apply --plan "$PLAN_TOKEN"
```

## Evaluation

### evaluation run

**Changes state and requests an evaluation trial.** Run one declared evaluation
slot, subject to the evaluation role, configured declaration and readiness gates.
This is controlled behavior evaluation, not ordinary product work.

```text
kegari-v0 evaluation run --workspace UUID --suite-digest DIGEST \
  --case CASE --variant VARIANT --profile PROFILE --repetition NUMBER \
  --expected-config REVISION --expected-control GENERATION [--request-id UUID]
```

All options except `--request-id` are required; there are no inferred slot defaults.

| Option | Meaning |
| --- | --- |
| `--workspace UUID` | Workspace UUID, not a display name. |
| `--suite-digest DIGEST` | Exact SHA-256 digest of the declared suite. |
| `--case CASE` | Declared case identifier. |
| `--variant VARIANT` | Declared case variant. |
| `--profile PROFILE` | Declared evaluation profile. |
| `--repetition NUMBER` | Positive repetition number. |
| `--expected-config REVISION` | Exact observed configuration revision. |
| `--expected-control GENERATION` | Exact observed control generation. |

The example variables name the corresponding declared slot and observed
revisions; they are not arbitrary values or permission overrides.

```sh
kegari-v0 evaluation run --workspace "$WORKSPACE_UUID" \
  --suite-digest "$SUITE_DIGEST" --case "$CASE" --variant "$VARIANT" \
  --profile "$PROFILE" --repetition 1 \
  --expected-config "$CONFIG_REVISION" --expected-control "$CONTROL_GENERATION"
```

## Remote targets

These commands manage client preferences in the invoking OS account's
`~/.kegari/clients.toml`. They reject `--target` and do not configure the remote
service. Remote access requires a configured HTTPS control endpoint and an
authorized identity.

### target list

**Read-only.** List the local client's saved targets.

```text
kegari-v0 target list
```

There are no command-specific options.

```sh
kegari-v0 target list
```

### target add

**Changes local preferences and contacts the target for verification.** Save a
named HTTPS target with an explicitly supplied runtime identity.

```text
kegari-v0 target add NAME --url ORIGIN --runtime UUID [--request-id UUID]
```

`NAME`, `--url` and `--runtime` are required. The URL must be an HTTPS origin,
without credentials, an application path, query or fragment. The client checks
TLS and verifies the expected runtime before saving. An identical saved target
is accepted; a conflicting saved name is refused. An observed runtime never
silently replaces the supplied UUID.

```sh
kegari-v0 target add studio --url https://host.example.ts.net \
  --runtime "$RUNTIME_UUID"
```

Use it with `kegari-v0 --target studio status`. There is no `target remove` command
in this version.

## Local service

These commands manage the control service on the invoking Mac and reject
`--target`. They are separate from orchestration commands and have no `--wait`
or `--timeout` options. The preview's native installation and activation remain
subject to operational qualification.

### service install

**Changes local installation state.** Write the per-user LaunchAgent and its
registration without loading or starting the service.

```text
kegari-v0 service install --binary PATH [--request-id UUID]
```

`--binary` is required and must be the absolute path to the protected, immutable
installed executable matching the runtime registration. Place the executable in
its final versioned location before initial setup. This command does not copy
the binary or provide an upgrade mechanism.

```sh
kegari-v0 service install --binary /absolute/versioned/kegari-v0
```

### service start

**Changes local service state.** Start the installed control host while
preserving persisted orchestration intent.

```text
kegari-v0 service start [--request-id UUID]
```

There are no additional command-specific options. A `start_requested` response
means the request was made; inspect `status` to establish readiness. Initial
setup's stopped orchestration intent remains stopped until an explicit `start`.

```sh
kegari-v0 service start
```

### service stop

**Changes local service state.** Shut down the control host and remove its active
launchd job after ownership checks and acknowledged shutdown.

```text
kegari-v0 service stop [--request-id UUID]
```

Orchestration must already be stopped and no active owned local work may remain.
Recovery blockers can prevent shutdown. Stopping the control host also removes
remote control availability; this is distinct from `stop`, which leaves the host
running. This command is not an uninstall command.

```sh
kegari-v0 service stop
```

## Output and exit codes

Human-readable output identifies the target, runtime and request, and includes
operation identity, state, result, reason and next action when available.

`--json` writes one complete UTF-8 JSON envelope followed by a newline to stdout.
Read results are assembled before output; incomplete paginated results are
refused rather than printed as a partial success.

| Field | Meaning |
| --- | --- |
| `api_version` | Control API version. |
| `request_id` | Identity of this request. |
| `runtime_id` | Runtime UUID, or `null` when no runtime is associated. |
| `config_revision` | Observed configuration revision, or `null`. |
| `observed_at` | Observation timestamp. |
| `ok` | Whether the command response is successful; not necessarily work completion. |
| `operation_id` | Accepted or observed operation UUID, or `null`. |
| `state` | Reported operation state, or `null`. |
| `data` | Command-specific result. |
| `error` | Error object, or `null`; includes `code`, `message`, `retryable`, `details` and nullable `next_action`. |

Parsing failures are sanitized and returned through the normal output format;
in JSON mode they also use the stdout envelope. `--json --help` and
`--json --version` return their informational text in `data.help`. Output failures
may emit a diagnostic on stderr.

| Exit code | Meaning |
| --- | --- |
| `0` | Successful inspection or accepted mutation, which may still be pending or draining. |
| `2` | Invalid command or input. |
| `3` | Configuration, prerequisite, I/O or another application error not mapped below. |
| `4` | Target/provider transport unavailable or a request timeout. |
| `5` | Authentication required, access denied or a missing secret. |
| `6` | Stale result/configuration/control generation, request or ownership conflict, identity/version mismatch, or expired snapshot. |
| `7` | Failed, partial, unknown or uncertain operation. |

The structured error code takes precedence over the state when selecting an
exit code. For example, an uncertain request with a transport timeout may exit
`4`; inspect the state and retained identities too. An observation deadline from
`--wait` is different from a request timeout and can exit `0`. A successful
`config check` can likewise contain blockers. Always inspect the result before
assuming work has completed or the runtime is ready.

## Further reading

- [Getting started](../README.md#getting-started)
- [Configuration guide](../crates/kegari-v0/docs/configuration.md)
- [CLI implementation and transport details](../crates/kegari-v0/docs/cli.md)

This reference describes the public CLI only. Internal process entry points and
the separate evaluation-inspection executable are outside its scope.
