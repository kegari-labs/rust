# Kegari platform

OS identity, protected executable inspection and native host helpers.

Import the selected interfaces directly from `kegari_platform`:

| Interfaces | Responsibility |
| --- | --- |
| `AccountHome`, `account_home`, `HostIdentity` | OS account/home lookup, host discovery and the existing state-root convention. |
| `BinaryIdentity` | Protected executable inspection and content revalidation. |
| `ProcessIdentity`, `ProcessObservation`, `ProcessProbe`, `NativeProcessProbe` | Process identity and bounded native observations. |
| `OwnedChild` | Actual owned child handles, streams, observation, signalling and cleanup. |
| `free_bytes`, `launch_client_browser` | Free-space queries and the OS browser-launch adapter. |

CLI and backend depend directly on this crate. It depends on `kegari-process`,
`kegari-digest` and `kegari-types`, plus the existing pinned serde, libc, UUID
serialization and Tokio process/time support. It has no backend dependency,
including in tests, and no direct configuration or storage dependency. UUID
generation is only a test dependency of this crate.

The backend `platform` module retains registration, owner records, runtime locks,
service planning/installation/lifecycle, handover, `BuildInfo` and installed-build
verification. It does not provide compatibility reexports of moved interfaces.
Application authorization and the assembled application's build identity remain
backend-owned.

## Ownership and native boundaries

Account-home lookup remains separate from host/boot discovery, so a remote-only
client does not gain a native host-discovery prerequisite. Existing error details,
`HostIdentity::state_root()` and platform-specific behaviour are preserved.
Executable inspection retains its path, ownership, permission, size and content
checks. Native queries preserve bounded inventory and incomplete-observation
refusal.

`checked_file`, native probing helpers and `OwnedChild` fields remain private.
Descriptive identities do not authorize adoption or signalling of recovered PIDs.
`OwnedChild` preserves its actual child-handle lifecycle; it is not merged with
`kegari-process::group::OwnedGroup`.

Browser URL validation remains in the CLI. The OS adapter continues through the
shared spawn interlock with its ten-second cutoff, cancellation behaviour and
existing error mapping. No browser is opened by ordinary verification.

## Verification and version

Run standalone checks with the pinned toolchain and existing cache:

```sh
cargo test --offline --locked --release -p kegari-platform
```

Use workspace-wide tests and Clippy for backend and CLI integration. The owned-child
cleanup fixture moves with its implementation. The two native platform qualification
tests retain their names, macOS guard and ignored status; the third native test
remains ignored in backend isolation. This refactor does not authorize native/live
qualification or add OS support.

Version remains `0.2.0-alpha.10`; executable behaviour, persisted/API schemas and
instruction assets are unchanged. No new alpha is packaged. See
[current progress](../kegari-v0/PROGRESS.md) and the
[verification report](../kegari-v0/validation/PLATFORM-EXTRACTION.md).
