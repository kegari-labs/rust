# Shared control host composition

This increment implements the shared authenticated dispatcher and host command
ownership. The complete native factory/supervisor and delivery runtime are still
being integrated; these component checks do not establish operational readiness.

Both transports call `HostDispatcher`. The accepting transport retains an OS UID
or verified Tailscale principal. The dispatcher rechecks its current operator
mapping; configuration and lifecycle mutations repeat that check inside the same
serialized gate that protects admission. Disabling HTTPS revokes an already
queued remote lifecycle request. Local service shutdown additionally requires a
Unix principal, durable STOPPED, drained local work and no owned background
configuration tasks. Its platform reservation prevents launchd from reversing
an acknowledged service shutdown.

Configuration commands persist a bounded command receipt and return its original
operation ID before waiting for provider/Codex metadata. The host owns the task
independently of the HTTP future. A deterministic derived operation ID identifies
the configuration effect; its receipt binds the parent ID and payload digest.
The five-minute task deadline records uncertainty, never permission to retry.
Identical requests inspect the existing operation; changed payload, principal or
secret input conflicts. Recovery cannot take over a still-owned background task.

Inspection pages bind the authenticated actor/principal, command and filters.
The full response retains the original configuration revision and operation state
through continuation, even if configuration changes between pages. Reads while
STOPPED create no mutation receipts or work. Start is reported as reconciling
until ownership/recovery actually opens admission.

`ObservationCache` performs only configured provider reads. It atomically swaps
complete projections and health; a failed refresh retains the old view with an
explicit failed health state. A workspace failure preserves its tracked parent
and required-child identities. Reference files contain canonical identities and
first-observed disappearance times, not issue bodies, prompts or workflow state.
Signed authorization/correction records are reconstructed from their actual
provider publications and verified against the host key/writer. Publication
removal or a change during reconstruction blocks the old authority. Identity-only
retention never grants execution or completion authority.

Local execution starts under `ControlHost::begin_effect`, which records Sent and
registers the owned process while admission is locked. Waiting occurs outside
that mutex. `complete_effect` rejects late passing evidence after a force fence
or authority change. Force cancellation requires the persisted fence and the
unreaped owned process capability. Metadata from an arbitrary PID cannot create
that capability.

Deterministic fixtures cover these paths. Native boot identity, Keychain,
launchd, actual sandbox/process supervision and live provider/agent behavior
remain separately qualified prerequisites; none was invoked by this increment.
