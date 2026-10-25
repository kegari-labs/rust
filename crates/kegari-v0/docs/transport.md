# Local and direct TLS control transport

`src/transport.rs` supplies actual Unix-socket and direct TLS Axum listeners with
per-connection identity, bounded identity subprocesses, certificate validation and
reload. The executable host must hold its control ownership lock before starting
these listeners. `start_unix` and `start_direct_tls` return independent `ServerHandle`s
with a shutdown method and a health watch. A remote prerequisite failure must not
prevent the host from retaining local control or authorized orchestration.

## Local socket

The socket is a leaf inside the already validated private control directory. Creation
never removes or adopts an existing path. Its mode is 0600, and the parent is protected
by `PrivateDir`. Every accepted connection obtains its UID through the OS peer credential
API. The UID is mapped through the current `control.local_users` configuration; JSON,
HTTP headers and process-provided usernames do not supply identity. Shutdown removes
only the exact socket inode created by that listener, preserving any replacement path.
Explicit shutdown first stops accepting connections and gracefully finishes admitted
HTTP responses within the ten-second request bound; an eleven-second overall drain
then aborts and reaps remaining connections. This also applies to direct TLS service
stop. Identity loss and certificate expiry still disable remote connections immediately.

Managed workers must be denied the control socket and remote endpoint by the separately
validated execution boundary. A same-UID process is not proof of a human operator, so
socket authentication alone cannot qualify unattended execution. A failed sandbox trial
remains a blocker; do not loosen socket permissions or bypass that trial.

## Direct tailnet TLS

The production listener binds only explicit configured Tailscale IPv4/IPv6 addresses
and the configured nonzero port. Startup checks the installed CLI and daemon profile,
local `Running`/online state, full DNS hostname and exact assigned addresses before
binding. It does not bind wildcard, LAN, public or loopback addresses. There is no
remote plaintext listener, proxy, Tailscale Serve configuration or fallback address.
A later rebind repeats identity validation. Existing bindings are rechecked periodically.

The accepted TCP peer is captured before TLS and carried to the API authentication
layer. Every request performs fresh verified identity lookup. The command is exactly
`tailscale whois --json --proto=tcp <accepted-peer-ip:port>`; IPv6 uses bracketed
address/port syntax. It must return the same directly assigned host address, matching
node/user identities and a nonexpired key. Tagged, shared or ambiguous identities and
subnet-route-only matches fail closed. The numeric `UserProfile.ID` selects the operator
mapping. `LoginName` is display information and never grants roles. The mapping is
reread after the asynchronous lookup so revocation during lookup takes effect before
dispatch. Configuration and installed verifier can be replaced atomically; an old
in-flight identity result cannot authenticate the replacement contract. Changing the
configured executable clears the old verifier until the host supplies the exact new
installed client; local authentication remains available. The serialized command handler still owns last-moment product authority checks.

The configured executable must be an absolute regular executable owned by the operator
or root and not writable by group/others. Only the fixed read-only version, status and
whois command vocabulary can be executed by this runner. It clears inherited environment
secrets, captures bounded stdout/stderr, applies a three-second deadline per command,
and kills timed-out children. Provider diagnostics are not copied into error messages.
No Tailscale install, enrollment, login, policy change, certificate issuance, update,
Serve or Funnel action is performed.

The supported JSON compatibility profile is Tailscale **1.104.2**, checked against the
CLI's `version --json --daemon` output and daemon status. Development/dirty builds and
other versions fail rather than assuming JSON compatibility. An operator-managed update
requires a reviewed profile update and qualification; Kegari does not install an older
or newer binary automatically. This host has no installed Tailscale executable, so real
identity/bind compatibility remains unqualified.

Configuration migration: this implementation has not been deployed. Any draft
`control.tailscale_users` keys using email/login text must be replaced by the verified
canonical positive numeric `UserProfile.ID`, such as `"42"`. Config parsing now rejects
login text, zero, signed or zero-padded IDs. Keep the mapping to its explicit operator ID,
Linear user ID and permitted roles; do not infer another account from a display name.

## Certificates and replacement

The operator provisions and renews a certificate and key for the full expected
`host.tailnet.ts.net` name. Certificate issuance may expose that DNS name in Certificate
Transparency. Tailscale enrollment/access rules and certificate issuance/renewal remain
operator-managed. Kegari does not change them.

TLS loading opens bounded regular files without following a final symlink. Ownership
must be the operator or root. The private key cannot be accessible to group/others;
certificates cannot be writable by group/others. The validator checks the complete DER
chain, actual validity windows, hostname and public trust chain, supported cryptography,
and leaf/private-key match. Production uses the pinned Mozilla trust roots. Tests supply
only their own disposable CA; there is no production insecure-certificate mode.

Replacement is polled once per second and accepted as a complete validated pair. A
mismatched, inaccessible or malformed replacement is reported while the previous pair
remains valid. At its actual expiry the TLS accept loop closes rather than extending its
lifetime or falling back to plaintext. With no valid pair the remote listeners are
removed; a later valid pair is accepted only after renewed address identity verification.
Local control has an independent handle and is retained. The TLS health watch records
enabled state, public certificate digest, actual expiry and a safe reload error.

The remote host must use `TransportAuthentication` with the configured Tailscale client
when constructing `ApiState`; the generic dispatcher/authentication traits exist for
host composition and deterministic fixtures, not worker-selected authentication plugins.
Host and runtime/protocol headers, browser-origin rejection, role checks, JSON body
limits and per-request deadlines remain enforced by the shared API router. Connections
also have a ten-second header/handshake limit and a finite concurrent-connection limit.
Malformed HTTP that cannot be parsed can close at the HTTP transport boundary; parseable
API failures use the shared JSON envelope.

## Evidence and integration limits

The tests exercise the production Unix listener with real OS peer credentials and
reqwest over the private socket; rejected remote prerequisites do not break that local
request. They exercise the validated production rustls configuration with a real TLS
handshake over an owned in-memory stream, and cover identity rejection, role revocation,
certificate hostname/chain/key/permission failures, invalid replacement and actual expiry.
No real tailnet listener was started and no host networking, Keychain or certificate
configuration was changed. Disposable test certificates/private keys are fixture data,
not application credentials.

The inspected official Tailscale source is tag `v1.104.2`, peeled commit
`e26db7a5797683ec99ad1516c40b628c7f2dfd71`: the
[whois command](https://github.com/tailscale/tailscale/blob/e26db7a5797683ec99ad1516c40b628c7f2dfd71/cmd/tailscale/cli/whois.go),
[version metadata](https://github.com/tailscale/tailscale/blob/e26db7a5797683ec99ad1516c40b628c7f2dfd71/version/prop.go),
[whois response](https://github.com/tailscale/tailscale/blob/e26db7a5797683ec99ad1516c40b628c7f2dfd71/client/tailscale/apitype/apitype.go)
and [node/user schema](https://github.com/tailscale/tailscale/blob/e26db7a5797683ec99ad1516c40b628c7f2dfd71/tailcfg/tailcfg.go).
The [official CLI guide](https://tailscale.com/kb/1080/cli) documents protocol-specific
whois and warns that status JSON can change, motivating the explicit profile.
Certificate validity inspection uses the documented
[x509-parser validity API](https://docs.rs/x509-parser/0.18.1/x509_parser/certificate/struct.Validity.html),
with rustls performing trust/hostname/signature validation.

Remaining qualification includes the operator's actual Tailscale CLI/daemon, direct
IPv4/IPv6 identities, enrollment/access policy, real public certificate chain and renewal,
worker denial of privileged endpoints, and executable host/client wiring. Component
tests do not establish any of those unavailable live facts.
