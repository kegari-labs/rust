# Broker call failure component validation

The test-only increment passed all-target compilation, strict Clippy, eight broker component tests, 95 integration tests and formatting. No native/model/provider qualification follows. The full suite has not been rerun on this source.

Frozen source: 696 inputs, `sha256:f26dfec202ba3840ab7c81dda8c6979ee0abb94aeef11c67d7b0195dc258105c`. Every command retained matching before/after source and toolchain observations (Rust/Cargo 1.99.1).

| Check | Result | Seconds |
| --- | --- | --- |
| All-target check | Passed | 16.462 |
| Clippy with warnings denied | Passed | 17.345 |
| Broker component group (8 passed) | Passed | 36.012 |
| Control storage/providers/worker (24/30/41 passed) | Passed | 30.773 |
| Formatting | Passed | 1.544 |

The three new tests use actual worker/broker/Control/protected journal components and an explicitly synthetic HTTP transport. They preserve the original request through caller cancellation, retain the actual returned-result digest when the final journal write refuses before writing, and refuse cleanup of an uncertain Sent receipt even after pending clears. ACK means acknowledged only, not remote readback. Generic runtime registration remains unavailable.

Independent review found BC-1: failure cleanup could delete unresolved evidence. V2 requires an unheld, explicitly settled original receipt and an exact retirement observation with no unresolved effects. Its separate rereview found no remaining material issue in that correction. The v1 artifact and finding remain preserved.

The shared fixture extraction preserves all 41 existing worker test bodies exactly. Earlier draft/review prose counted 36 because its regex missed five parameterized `#[tokio::test(...)]` attributes; there are no added nested helper tests. Only the relocated pinned_git import and integration include change. The new crate alias and module hook are test-only.

The lost-response regression deliberately retained its synthetic uncertainty records at `/var/folders/s3/gnkb4nw50msf40dxc1jrpsyh0000gn/T/.tmpAZP6yJ` before introducing the effect. It did not clear the hold, fabricate reconciliation or delete that directory. Both owned execution sessions finished; no validation process is retained by this checkpoint.

No production behavior, scorer, shipped fixture, permissions or coverage-ledger status changed. Prior full-suite 954/0/2 results belong to the preceding 694-input source and are not a new full-suite result. The canary implementation and E07 source preparer remain separate uninstalled drafts.

Machine evidence: [checks](broker-call-failures-checks.json), SHA-256 `46eb7765bdd6cdd8694d1998580fb62a91f515902d87f3824525229d3df4f54c`. Exact commands, logs, source hashes, review references and retained-fixture provenance are recorded there.
