# Kegari execution

This crate owns managed environments, filesystem isolation, resource binding,
native command launch, output collection, cancellation and joined cleanup.
`journal` owns protected process records and actual registered process guards.
Application admission and interpretation of results remain backend responsibilities.

## Ownership and admission

Import environment and command interfaces from `kegari_execution`; import journal
records, `HostProcessIdentity`, `ProcessJournalStore` and `SpawnGuard` from
`kegari_execution::journal`. Shared leases are `kegari_types::control::Lease`.
There are no backend compatibility reexports.

`ProcessJournalStore::begin` validates a binding, acquires real registered lock
descriptors and persists a new `Starting` record. It establishes persistence and
resource ownership, not application authorization. It cannot reconstruct a guard
or adopt a process from retained records. Recovery observations remain descriptive.
The backend `process_records::ProcessJournal` retains worker-only admission and
validation against the sealed host permit; it does not expose its underlying store.

`start_journaled_command` accepts a trusted, synchronous, one-shot journal factory
and `CommandLaunchOptions`. The factory runs after qualification, rendering and
command construction at the existing journal-creation point. It is consumed
before spawn and never retained in the supervisor or cleanup. Backend worker and
bundle adapters retain the actual permits and construct exact journal bindings.

`start_guarded_command` checks a supplied guard against the exact command.
Foreground, invocation and bundle modes preserve their distinct output budgets
and deadlines. Only an explicitly selected bundle service uses graceful service
shutdown; descriptive guard ancestry does not select that behavior. Limits and
options grant no admission authority.

`OwnedCommand` and its cancellation handle retain the actual child, owned group,
concrete journal guard and output collectors. Closure requires owned exit evidence,
verified journal retirement and joined output. The shared process interlock still
spans descriptor release and existing-lock observation. A surviving inherited
holder preserves uncertainty even if it later releases its descriptor.

## Application identity and compatibility

`IsolationPlan::validate`, `BoundManagedExecution::automatic`,
`automatic_policy_digest` and `revalidate` accept `runtime_build: &str`.
Production callers supply the backend's current `BUILD_ID`; validated isolation
retains it privately through cwd derivation and resource revalidation. This crate
has no build script or competing application identity.

Plans remain schema 1, managed domains schema 5, observations schema 1 and process
records retain schemas 1–3. Equal inputs, including build identity and filesystem
observations, retain the same bytes and digests. A changed application build still
invalidates retained managed domains as before.

## Backend integration and retained restrictions

The backend `execution_host` module contains only worker, invocation and bundle
adapters. Backend `execution` retains application commands, check bundles, protected
artifacts and evidence. Durable permits, execution reservations, receipts and
recovery policy retain their current owners. Native Git/Codex adapters use the
concrete moved guards; their operation policy remains backend-owned.

`process_tree_unverified` and `managed_profile_unqualified` refusals are unchanged.
This extraction does not qualify or enable repository command execution, prove
arbitrary detached-descendant containment, or add toolset-specific configuration.

## Dependencies and checks

Production workspace dependencies are types, digest, platform, process and storage.
There is no backend dependency, including in tests, or direct configuration,
provider or Codex dependency. Tokio owns the existing process/I/O supervision;
UUID generation remains in backend adapters and disposable tests.

Run from the workspace root with the pinned toolchain:

```sh
cargo test -p kegari-execution --release --offline --locked
cargo test --workspace --release --offline --locked
cargo clippy --workspace --all-targets --offline --locked -- -D warnings
```

See the [final-pass report](../kegari-v0/validation/EXECUTION-LIFECYCLE-EXTRACTION.md)
and [historical environment report](../kegari-v0/validation/EXECUTION-ENVIRONMENT-EXTRACTION.md).
