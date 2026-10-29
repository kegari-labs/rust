# Durable host-process and Codex session references

This page describes retained ownership for Kegari-created app-server and Git
processes. Codex owns its internal repository commands under [SPEC 1.5](../SPEC.md);
those commands do not require Kegari process journals or individual admission.

Native worker startup creates a versioned process record before starting either
the pinned provider's `--version` probe or its app-server. The record binds the
operation/effect, lease, canonical work, immutable scope, executable bytes,
arguments digest, private cwd and host identity. Startup and cancellation-handle
registration occur inside the current serialized admission gate. Waiting and
protocol reads occur outside that gate.

The child inherits one descriptor for the actual held guard lock. Recovery opens
the recorded device/inode without creating a replacement and checks exact OS
birth/boot/executable identity where available. Saved PIDs remain observation
references; they never become owned handles or signal authority. Replaced,
missing, corrupt or oversized records block recovery. Per-operation enumeration
is bounded to 4,096 process records and each record to 64 KiB; the global inventory
is bounded to 65,536 operation directories. Concurrent claims recheck durable
record existence after acquiring their guard, so releasing a guard does not
permit replay of the same effect.

A metadata probe may exit before live identity inspection. It qualifies a version
only after the actual spawned Child is reaped following owned-group cleanup,
bounded output matches the pinned version, and executable identity is rechecked.
An app-server whose live identity cannot be captured receives no protocol; its
actual cancellation handle remains registered for termination and joining.

The journal deliberately reports `all_descendants_exited: false`. An inherited
lock can prove that a holder remains; its absence cannot prove that a detached
child which closed descriptors exited. Group cleanup and pipe EOF also do not
establish whole-tree containment. Recover supported Codex session/lifecycle facts
without adopting arbitrary descendants or blindly replaying interrupted commands.
A restarted host preserves unresolved admissions instead of inferring exit from
an empty in-memory registry. The replaced executor's blanket qualification refusal
is not a requirement on the new Codex-backed path.

Session recovery separately reads the exact admitted instruction, permission,
profile, account, work and generation bindings, then contiguous bounded journal
events. Earlier provider intervention or unavailable input is retained even if a
later checkpoint omits it. The host receives each durable session checkpoint
before further protocol processing and serializes provider/work holds against
new admission. No profile change, correction generation, later ordinary session
or retry grant constitutes provider intervention resolution.

Deterministic tests exercise inherited descriptors with owned disposable child
processes, missing/replaced guard inodes, quick exits, concurrent claims, bounded
records, forced provider cancellation and intervention retention. Fixture process
metadata is explicitly injected. These tests do not qualify native boot identity,
provider authentication, live model behavior or descendant confinement.
