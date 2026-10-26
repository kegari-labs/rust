# Native loader bootstrap — alpha.7

The corrected production sandbox passed the separately approved NQ-02C native
foreground test once, covering all three fixed cases below. The affected local
checks passed 97 tests, with one native test ignored in the ordinary run.
Milestone 0.2.0 and operational qualification remain incomplete; the public
`managed_profile_unqualified` refusal remains enforced.

| Native case | Observed result | Closure evidence |
| --- | --- | --- |
| Direct Bash: output, redaction, allowed scratch write and denied protected read/write | Exit 0; 57 ms; expected redacted stdout and diagnostic stderr; scratch result and protected sentinel verified | Original group exited, journal closed, output joined |
| Direct Bash: unsuccessful exit | Exit 7 preserved; 50 ms; empty output | Original group exited, journal closed, output joined |
| Owned `/bin/sleep 5`, command timeout 100 ms | Timed out; SIGKILL (9); 141 ms; empty output | Original group exited, journal closed, output joined |

All cases recorded no output truncation and `all_descendants_exited: false`.
Group closure does not prove termination of escaped, unregistered descendants.
The native test used a synthetic disposable lease with the production launcher,
sandbox, supervisor, collectors and process journal. It did not exercise real
runtime admission or issue qualification. Its final marker explicitly records
`qualification_issued: false`.

The narrow correction gives the seven existing fixed OS Cryptex grafts explicit
existence access and gives their eight unique literal ancestors directory read
and existence access. Those ancestors include `/`; none receives a recursive
content grant, executable mapping, execution or write permission. Reading a
literal directory permits inspecting its entries. Forbidden roots explicitly
deny existence checks as well as read/write/map/exec. Network, Mach, general
profile imports and unrelated grants are unchanged. Static regressions check
the exact grant sets and continued public qualification refusal.

The installed Apple dyld-support policy informed this correction. It is a private,
changeable interface; a pass on this host does not qualify another OS/profile.
The [NQ-02A](NATIVE-MANAGED-COMMAND.md) and
[NQ-02B](NATIVE-MANAGED-RECHECK.md) failures remain historical evidence. NQ-02B's
matching crash report places its abort in libignition/dyld bootstrap, but the
exact denied operation in either earlier failure remains unproven. This pass
must not be used to assert a more specific historical cause.

The fixture now retains evidence on success and failure, correlates launch
operation/effect IDs with journal rows, and retains its original waiter through
bounded timeout cleanup. A nine-second independent case timeout still fails the
test; cancellation plus at most two seconds of settlement observation cannot
turn it into success. Independent review found and corrected a start-diagnostic
write that could panic while the command was live. Its assertion now occurs
after settlement or bounded cleanup. NQ-02C did not exercise that independent
timeout branch; its normal 100 ms command-timeout case did execute.

NQ-02C used exactly one explicit approval, now consumed, with a 30-second outer
process timeout. Test and wrapper both exited 0; one selected test passed, zero
failed, 544 were filtered, and the outer timeout did not fire. Wrapper elapsed
time was 0.923 seconds. All 22 retained fixture files were hash-verified.
NQ-01B remains separately unapproved and unexecuted. No setup, Keychain access,
service/coordinator activation, provider action or existing-process action was
included.

- Native binary: `target/native-qualification/nq-02c-d2179900672c/managed-command-test`
- Native SHA-256: `d2179900672c79ebcd3217ce556ec0b1c2d4de57a240cf09fe703fb4be105c54`
- Retained fixture: `/private/tmp/kegari-nq02c-959z2ztz/.tmpnjzxmt`
- [Native request, result, case observations and hashes](native-loader-bootstrap-checks.json)

The release executable is:

`target/prereleases/kegari-v0-0.2.0-alpha.7-aarch64-apple-darwin-be94ce252874/kegari-v0`

- SHA-256: `be94ce25287486f2fc147c0901fce5946a0ee041c91ccc68074f3dafbc18980e`
- Build identity: `sha256:76b7b0d07c6c4dd047733a6b6cba94053493631362ac2997e7c34d2efa8c5c38`
- 41,263,264 bytes, mode 0555, release profile, aarch64-apple-darwin.
- Rust/Cargo 1.99.1. The final 723-input test manifest matches the native pin;
  the release artifact records its separate 325 build inputs.

Validation commands use `RUSTUP_TOOLCHAIN=stable` and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, with that stable alias verified as
the pinned Rust/Cargo 1.99.1. Exact commands and hashed logs are retained in
[machine validation history](loader-bootstrap-checks.json).

| Check | Result and source boundary |
| --- | --- |
| Release library `isolation::` tests | 21 passed, zero failed, one native ignored. Before the final test-only diagnostic-order correction; production and ordinary test source unchanged afterward. |
| Release `codex_protocol`, `execution`, `main`, `housekeeping` integration tests | 76 passed, zero failed; final source |
| Final native library compile | Passed; no tests executed by compilation |
| All-target clippy with warnings denied; formatting | Passed |
| Contract inventory | Passed; 503 clauses, 136 named requirements, no promotions |
| Release build and five staged inspections | Passed; help/version/setup-help/build/handover metadata; minimal environment, closed stdin, empty disposable cwd, no cwd writes |
| Separately approved NQ-02C | One test containing the three native cases passed on the final source |
| Full regression baseline | Alpha.6's 1,097 passes, zero failures, three native ignores across 40 groups reused only for unchanged areas; no alpha.7 full-suite run claimed |

Independent source review checked the exact sandbox grants and their negative
boundaries, fixture ownership and failure ordering. Independent package review
checked the binary/wrapper pins, all 723 source entries, exclusive attempt marker
and the required selected-test plus three-case completion evidence. The earlier
full-suite failures and native failures remain in their original reports.

An independent final artifact/evidence audit matched all 48 referenced hashes,
325 build inputs and 723 final test inputs, recomputed the build identity, and
checked the actual three-case launch/journal/output records and all five staged
inspections. The human report, README and progress record agreed with those
results; no material inconsistency was found.

This is an inspectable development build, not an installation or whole-binary
reproducibility claim. Instruction assets remain version 3; handover metadata 3,
process compatibility 3, managed-execution schema 2 and config/receipt schema 1
are unchanged. Native services, scoped networking, the complete offline
Rust/Cargo profile and later recovery, external/visual evidence, handover,
evaluations and real/human qualification remain open. The next implementation
package is the [closed offline Rust/Cargo contract](../docs/managed-qualification-design.md).
Full descendant coverage remains required before qualification on untrusted
external context, including public pull requests.
