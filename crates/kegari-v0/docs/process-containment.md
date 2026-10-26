# Process containment: requirement and current feasibility

Kegari must know which local processes belong to an operation, stop only those
processes on cancellation, and establish that none can keep executing before
accepting complete local shutdown. The missing property is descendant lifetime
ownership. Filesystem/network/tool restrictions remain a separate requirement.
This note records the 2026-10-26 investigation at implementation checkpoint
`a217df8`; it does not qualify a native backend or remove the current refusal.

## Why the existing process group is insufficient

A shell or test runner can fork a child, the child can create another session
or process group and close inherited output streams, and the parent can exit.
The child may still write into the operation's allowed directory. The existing
finite `tests/process_group.rs` regression demonstrates exactly that late write.
It establishes a process-group limitation, not a filesystem-sandbox escape.

`src/isolation.rs::repository_command_prerequisite` currently rejects repository
commands with `process_tree_unverified`. Successful group cleanup and EOF cannot
remove that refusal. PID enumeration alone also cannot establish complete
membership or safely signal a process whose numeric PID has been reused.

| Feature | Required use of descendant ownership |
| --- | --- |
| Workflow hooks and local/integrated checks | Execute under the operation's role, then establish actual completion before accepting results. |
| Codex app-server and Git subprocesses | Carry the same stronger proof through provider/Git cancellation handles and worker completion. Existing group results are insufficient. |
| Pause, stop, timeout and cancellation | Preserve graceful drain; fence mutation authority before forced termination; distinguish requested cancellation from verified exit. |
| Restart and recovery | Preserve uncertain ownership and prevent duplicate replacement work; persisted PIDs, group-exit records and empty memory do not recreate live authority. |
| Candidate/evidence sealing and delivery gates | Prevent an outstanding process from changing artifacts after they were treated as final. Remote CI/deployment outcomes require separate provider evidence. |
| Cleanup and handover | Prove local shutdown before removing owned directories or replacing the coordinator. |
| Actual-agent evaluations | Use the production lifecycle boundary and retain honest interrupted/unknown outcomes. |

Compilation, help/version output, configuration parsing and deterministic
fixtures can proceed while native qualification is incomplete. Independent PR
review remains read/feedback-only and must not gain repository execution.

## Contract and implementation boundary

SPEC §§6.1, 7.1, 11.2–11.5 require admission before launch, independent resource
isolation, verified-owned termination, preserved uncertainty and safe recovery.
The implementation needs enrollment before executable work, identity-safe
signalling, complete membership through fork/exec/reparent/session changes,
event-loss detection, and a defensible final observation boundary.

The specification does not require the kernel to kill every descendant after
simultaneous death of every observer. That failure may remain an explicit
reconciliation blocker: no successful shutdown or completion can be inferred,
and replacement work or cleanup for the affected operation remains blocked.
Existing resource isolation must continue to enforce
its boundary independently. This allowance does not waive actual force-stop
termination or permit a live observer to declare success from incomplete data.

## Candidate and unresolved questions

The installed macOS 27 SDK and Apple's current documentation expose
[`es_new_descendants_client`](https://developer.apple.com/documentation/endpointsecurity/es_new_descendants_client(_:_:)).
It observes the calling process and its descendants, requires Apple's Endpoint
Security entitlement, and does not require root or TCC approval for this client
type. Apple's DTS guidance confirms visibility across reparenting/session changes
and recommends audit-token signalling for known descendants. Signing requires
the client to be packaged in a bundle. See
[DTS lifecycle guidance](https://developer.apple.com/forums/thread/848882).

[`es_sync_client`](https://developer.apple.com/documentation/endpointsecurity/es_sync_client(_:_:))
provides a queue-processing barrier; its callback also runs on client destruction.
It is not a standalone complete-exit certificate. The unresolved design work is
how to establish final closure under concurrent creation, missing notifications,
identity transitions and observer failure. Fast handlers, multiple clients and
successful ordinary runs must not silently become a guarantee of losslessness.

`libproc.h` labels its interfaces private/subject to change, while Apple DTS
specifically endorses `proc_signal_with_audittoken` and
`proc_terminate_with_audittoken` as reasonable exceptions. Availability and
behavior still need qualification on the supported host. Identity-safe signalling
does not discover a descendant whose creation was never recorded.

Apple DTS also confirms that ES restrictions disappear with the client and that
macOS has no kernel-owned inherited boundary that terminates all members when
every custodian disappears. Those stronger lifetime guarantees must not be
attributed to this candidate. See
[client loss](https://developer.apple.com/forums/thread/850253) and
[custodian lifetime](https://developer.apple.com/forums/thread/851230).

No supported entitlement-free substitute was established by this investigation.
Process groups/launchd group cleanup do not cover detached groups; ancestry scans
have races; denying subprocess creation breaks ordinary repository tools;
resource sandboxes do not themselves prove exit. This is a bounded feasibility
finding, not proof that no future platform facility could satisfy the contract.

## Resource and qualification state

The user reports no existing Endpoint Security entitlement and is renewing Apple
Developer Program membership. Membership is distinct from Apple's restricted
entitlement approval. Apple identifies non-security helper supervision as an
intended use case, but that is not approval for Kegari. The public request route
is [System Extensions and DriverKit](https://developer.apple.com/system-extensions/).
No request, signing, installation or client activation has been performed.
No approval timeline is established; see
[Apple's request-status clarification](https://developer.apple.com/forums/thread/841639).

The existing platform test binary was prepared successfully, without running
tests, using `cargo test --locked -p kegari-v0 --test platform --no-run
--message-format=json` with the recorded stable toolchain and private Cargo cache.
Cargo reused the current artifact (`fresh: true`):

- Executable: `target/debug/deps/platform-8a42d593f3c5ceff`.
- SHA-256: `d45a9884f1e42762e6a0a2e9235ed318e128e4b353968d9b93e741b57b2a17fb`.
- Build record: `/private/tmp/kegari-v0-native-prerequisite-build-1.jsonl`.

The proposed first native pass remains unapproved: one host/birth-identity test,
then, only on success, one owned-child signalling test using `/bin/sleep 30`.
Neither establishes descendant containment. The second test's two-second poll
is not a hard whole-test deadline: Drop performs a direct-child kill and blocking
wait. A reviewed outer execution/cleanup bound and a permitted host environment
must be supplied before the exact request is ready to execute. Existing sandbox
and boot-identity denials must not be retried unchanged or bypassed.

The next engineering decision is whether the candidate can support the required
live-interval closure argument and a deployable entitled helper. Keep the refusal
until implementation and actual qualification supply that evidence; preserve
independent authorized work when a resource or platform decision is pending.
