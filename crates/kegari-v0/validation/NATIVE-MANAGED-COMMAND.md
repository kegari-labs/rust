# Native managed-command qualification — NQ-02A failure

The user approved one bounded run of NQ-02A on 2026-10-26. It was executed
once outside the sandbox with its prepared 30-second wrapper and pinned binary.
**The first case failed; native managed-command qualification remains incomplete.**
The permission is consumed. No repeat, setup, service, Keychain or provider action
is authorized by that approval. NQ-01B remains a separate pending request.

| Evidence | Result |
| --- | --- |
| Binary SHA-256 | `a989ca4d2520ca9a10a858bee23a043db87e5052eebc3eb79bea3f030c375227` |
| Test | `isolation::native_tests::managed_foreground_native_exit_output_and_timeout_are_journaled` |
| Selected argv | `--ignored --exact <test> --test-threads=1 --nocapture` |
| Test result | 0 passed, 1 failed, 0 ignored, 507 filtered; exit 101 |
| Wrapper | No timeout; 0.744292207993567 seconds |
| First assertion | Expected `Some(0)`, observed `None` at `isolation_native_tests.rs:143` |
| Remaining cases | Shell exit 7 and sleep timeout were not reached |
| Retained journal | Schema 2; `group_exited`, null exit code, original owned-group identifier, guard release verified |
| Retained filesystem | Scratch empty; protected sentinel unchanged |

The wrapper completed, and the fixture retained its failed evidence under
`/private/tmp/kegari-nq02a-n_3qkjgx/.tmphqx7PS`. The exact request, executable,
attempt marker, result and hashed test stdout/stderr remain under
`target/native-qualification/nq-02a-a989ca4d2520/`. The wrapper's successful exit
does not mean the selected test passed. [Machine evidence](native-managed-command-checks.json)
preserves the selected-test outcome and artifact references.

The test's `command.wait()` returned an outcome, but its assertion preceded
persisting the command stdout/stderr and other observations. A null numeric
exit code does not identify a signal or a particular sandbox denial. The durable
group record is narrow lifecycle evidence, not successful command execution or
arbitrary-descendant closure. No stale PID was signalled or adopted during diagnosis.

Read-only inspection found launch prerequisites omitted from the fixture/profile:
the generated Seatbelt policy lacks explicit executable-mapping grants, which
installed Apple profiles grant separately; `/bin/sh` is a selector that re-execs
the shell chosen by `/private/var/select/sh`. The installed `sh(1)` manual
documents this behavior, and the symlink on this host selects `/bin/bash`.
Only `/bin/sh` and `/bin/sleep` were declared. These findings justify corrections,
but the failed evidence does not establish which denial caused this termination.

The alpha.6 source now preserves per-case diagnostics before assertions, pins
Bash directly, adds scoped loader/executable mappings and tests policy boundaries.
The separately prepared NQ-02B request pins the corrected foreground fixture at
`target/native-qualification/nq-02b-ac6cf23ac60b/managed-command-test`, SHA-256
`ac6cf23ac60b314bd3839880d7ffb1ab026a26ff0a2612e30bf66e403e78a4c9`.
It was subsequently approved and executed once, then failed in loader bootstrap
with SIGABRT. See the [separate NQ-02B result](NATIVE-MANAGED-RECHECK.md). That
pin predates the later host-launch/guard-retirement correction and cannot establish
native qualification of the final alpha.6 source. Keep the public managed-profile
gate closed. NQ-01B remains separate and unexecuted.
