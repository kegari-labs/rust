# Kegari storage

Owner-only, descriptor-relative filesystem access, durable protected records,
atomic writes, lock primitives and retained immutable-file identity.

Import `PrivateDir`, `WriteBoundary`, `SectionLock`, `SharedLock`, `ExclusiveLock`,
`RetainedPrivateFile` and `decode_record` directly from `kegari_storage`.
There is no `kegari_v0::storage` compatibility reexport.

CLI and backend depend directly on this crate. It depends on `kegari-types` and
`kegari-digest`, with no backend, asynchronous runtime or process-launch dependency.
Record schema 1, digests, bounds, errors and durability ordering are unchanged.

`RetainedPrivateFile` is opaque and can only be obtained through actual exclusive
creation; it is not reconstructed from serialized metadata. Directory revalidation
and record decoding are checks, not execution/admission authority.

`ExclusiveLock::raw_fd`, `identity` and `inherited_file` are low-level trusted
integration APIs. A borrowed descriptor must remain live and must not be closed
or explicitly unlocked. Duplicates share the locked open-file description and
retain close-on-exec. Backend `RegisteredGuard` continues to acquire, register,
retire and close process guard descriptors under its single spawn/retirement
interlock. Storage neither acquires that mutex nor grants spawn authority.
`SectionLock` and `SharedLock` keep their lexical unlocking semantics;
`ExclusiveLock` keeps close-only, last-owner lifetime semantics.

Domain records, recovery and admission policy, process supervision and the spawn
registry remain backend-owned. Version remains `0.2.0-alpha.10`; this extraction
is not a new release or native qualification.
