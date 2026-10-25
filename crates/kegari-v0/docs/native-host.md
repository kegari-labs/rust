# Native control host composition

`host::run_native` accepts a mandatory `HostRuntimeFactory`. Its returned
`RuntimeBackend` supplies the actual workflow controller and supervisor driver;
the host does not install an empty or permissive scheduler. The root runtime
coordinator and final `app::run_control_host`/pending main entrypoint are still
being integrated. Native service startup has not been executed or qualified.

Startup uses the OS account/boot identity and exact immutable executable,
recovers the existing registration under `control.lock`, and loads the validated
configuration and saved intent. It never bootstraps missing configuration or
secrets. Identity, build, configuration, shutdown-marker and lock conflicts fail
closed before listeners or orchestration start.

The private Unix listener uses the configured local operator map and
`kegari.local` authority. Stale-socket recovery first proves the current registered
control owner, requires an owner-only socket inode, confirms connection refusal,
then rechecks that exact inode. Live endpoints, ordinary files, links and
ambiguous errors remain untouched. Native process identity qualification is
separate from the injected identity fixtures used to test socket handling.

Existing-Keychain failure produces an explicit prerequisite-blocked backend.
Local status, configuration and incomplete read-only projections remain
available; retained parent/required-child identities are preserved and never
treated as authorization. Runtime mutations and scheduling return the actual
prerequisite error. The host retries the same existing key with bounded backoff;
it never creates a replacement key, changes account or falls back to plaintext.
Once dependencies are available it constructs the real configured backend and
retries only a failure that occurred before any orchestration owner was started.
Restoration does not reset attempt budgets, interventions or uncertain effects.

Remote control starts independently through the direct TLS transport. Invalid
Tailscale, certificate or bind prerequisites disable remote access while the
local listener remains available. HTTPS contract changes replace the listener
and exact Tailscale verifier; in-flight identity checks remain revision-bound.
The transport watches certificate replacement/expiry and the host exposes its
health. There is no plaintext, proxy, alternate-account or alternate-target path.

SIGINT/SIGTERM request supervisor interruption without changing saved desired
intent. With an active owner, the supervisor durably suspends admission and
fences operations before signalling owned work; it then observes local exit
within declared bounds and preserves unresolved effects. Signals do not write
the explicit service-stop reservation. The `service stop` API instead requires
STOPPED and no owned local work, persists that reservation and gracefully drains
the response before listener shutdown. A restart cannot silently clear it.

Deterministic tests cover protected stale socket handling, prerequisite-blocked
inspection, pre-spawn prerequisite restoration, idle/scheduled host interruption,
and saved-intent preservation. They do not exercise login/lock, native boot
ownership probes, installed launchd services, real Keychain credentials, tailnet
identity, TLS provisioning or an actual model.

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test -p kegari-v0 --lib --test supervisor --test platform
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy -p kegari-v0 --lib --test supervisor --test platform -- -D warnings
```

The private Unix listener starts before deferred prerequisite construction.
Existing-key and backend observations run as an owned, bounded future alongside
signals, status and supervisor monitoring. A missing/locked key remains an
explicit prerequisite error; the host never creates a replacement during recovery.
TLS monitor installation compares the current full configuration revision under
the control lock before replacing authentication settings and again before
retaining a newly started listener. A stale listener is shut down and cannot
restore an earlier operator-role mapping. A regression checks this against the
actual transport authenticator.
