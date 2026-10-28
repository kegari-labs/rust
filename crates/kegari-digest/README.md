# Kegari digest

This crate owns `bytes_digest`, `canonical_bytes` and `canonical_digest`.
Import them directly from `kegari_digest`; `kegari_v0::digest` no longer exists.
The implementation and formats are unchanged: byte SHA-256, sorted JSON object
keys, meaningful array order, positive schema prefixes and rejection of floating
point identities. Hashing performs no I/O or authorization.

Dependencies are the existing pinned serde/JSON/SHA-256/hex libraries and
`kegari-types` for errors. There is no backend dependency. Configuration revision
calculation remains a backend function using this shared implementation.

Version remains `0.2.0-alpha.10`. Run workspace-wide checks from the workspace
root; see [progress](../kegari-v0/PROGRESS.md) for actual results. This extraction
creates no release package or operational qualification.
