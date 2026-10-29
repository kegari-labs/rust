# Kegari process

Guarded standard/Tokio spawning and owned process-group lifecycle handling.

| Module | Responsibility |
| --- | --- |
| `kegari_process::spawn` | Single spawn/retirement interlock, descriptor registry, registered guards and spawn wrappers. |
| `kegari_process::group` | Owned process groups, observation, termination, reaping and owned exit evidence. |

Import these modules directly; the backend no longer exposes `process_spawn` or
`process_group` compatibility modules. The backend depends on this crate. The CLI
reaches platform operations through `kegari-platform` and has no direct process
dependency.

This crate depends on `kegari-storage`, `kegari-types`, Tokio's `process` feature,
libc, UUID generation and `serde_json` for existing diagnostic details. It has no backend dependency, including in tests.
Shared process-journal mechanics currently live in `kegari-execution`.
Control permits, journal admission, recovery policy, registration, service
management and application supervision remain in the backend. Preserve this
crate's actual Codex/Git/control callers during the [SPEC 1.5](../kegari-v0/SPEC.md)
migration. Its presence does not require Kegari to supervise every internal Codex
command; Codex owns repository execution and supported terminal management.

## Ownership and interlock

`SpawnBoundary` and `RegisteredGuard` expose existing integration operations with
private state. The descriptor registry and mutex are private and have one
implementation. Guard acquisition, registration, retirement and descriptor closure
use this same interlock. Execution journal retirement continues to hold it across
guard release and observation of the original lock inode.

The spawn wrappers preserve deadlines after interlock acquisition, pre-exec hook
ordering, stale-hook disarming, close-on-exec handling and poisoned-lock refusal.
They are trusted process mechanics, not application admission or sandbox policy.

`OwnedGroup` remains bound to an actual owned child. Its identity constructors and
fields stay private; `bind_child` checks an existing association rather than
constructing one from persisted identity. `OwnedGroupExit` is privately constructed
and non-deserializable. Observation, group cleanup and reaping preserve the
original child's lifetime and do not adopt recovered PIDs.

Group cleanup does not prove containment of descendants that detach into another
session/group. The existing managed-workload limitation remains unchanged.

## Verification and version

Run the standalone crate with the pinned toolchain and existing cache:

```sh
cargo test --offline --locked --release -p kegari-process
```

Use focused integration coverage when changing retained host callers. Workspace-wide
checks belong to later stabilization under the [development policy](../../docs/development.md#feature-first-development-policy).
Existing disposable process fixtures remain useful; native/live qualification is
separately restricted. The retained native platform qualification tests remain
ignored; the obsolete custom-command qualification test was removed with that executor.

Version remains `0.2.0-alpha.10`. Current progress records the active redesign
and its changed schema/asset identities; the extraction report retains its
original evidence. No release is packaged by these source changes. See [current progress](../kegari-v0/PROGRESS.md) and the
[verification report](../kegari-v0/validation/PROCESS-EXTRACTION.md).
