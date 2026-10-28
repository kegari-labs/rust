# Kegari control API

`kegari-control-api` owns authenticated control transport and request handling.
It retains version `0.2.0-alpha.10`, API schema `1` and the existing wire formats.

| Module | Responsibility |
| --- | --- |
| `api` | Command validation/classification, authentication and dispatcher contracts, HTTP routes, request limits and response handling. |
| `identity` | Existing serializable `Actor` and `Principal` values and the basic claimed-role check. |
| `transport` | Private Unix and direct TLS listeners, Tailscale peer identity, certificate verification and authentication/listener replacement. |
| `snapshot` | Immutable actor-bound snapshot capture, expiry, pagination and verified reassembly. |

Import these modules directly as `kegari_control_api::{api, identity, transport,
snapshot}`. The previous backend modules and `control::{Actor, Principal}` paths
are removed without compatibility reexports. Shared command, response, error and
pagination data remain in `kegari-types`.

`Actor` is an existing serializable value, not an opaque authentication grant.
`Actor::require` checks identity presence and claimed role membership. The
backend still verifies the current principal mapping, configured roles and
permission fences. `api::Peer` remains transport-supplied and non-deserializable;
request headers and bodies cannot create the accepting connection's identity.

`api::evaluation_trial_identity` preserves the existing deterministic trial-slot
identity. It grants no execution authority. Its private digest-format check is
checked against the retained backend evidence validator; it does not import
backend evidence policy.

The crate depends on config/digest/process/secrets/storage/types, with no backend
or controller dependency, including test dependencies. Transport continues using
the shared guarded process-spawn implementation. No optional client/server
feature split is introduced in this extraction.

The backend remains the owner of the assembled application build identity. This
crate has no competing build ID and does not change schemas, command behavior,
process roles, instruction assets or native/live qualification status.

From the workspace root, with the pinned toolchain and dependency cache:

```sh
cargo test --offline --locked -p kegari-control-api
cargo test --offline --locked --workspace --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests use fake identity/provider components and disposable socket, process and
TLS fixtures. They do not invoke installed Tailscale, real certificates, Keychain
items or live provider endpoints.
