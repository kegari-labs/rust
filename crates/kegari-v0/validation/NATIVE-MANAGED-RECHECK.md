# Native foreground recheck — NQ-02B failure

The user approved one bounded NQ-02B run. It executed once after the local Cargo
checks finished. **The first Bash case aborted; native foreground qualification
remains incomplete.** The single-use approval is consumed. NQ-01B remains a
separate unexecuted request. No setup, Keychain, service or provider action ran.

| Evidence | Result |
| --- | --- |
| Binary SHA-256 | `ac6cf23ac60b314bd3839880d7ffb1ab026a26ff0a2612e30bf66e403e78a4c9` |
| Selected test | `isolation::native_tests::managed_foreground_native_exit_output_and_timeout_are_journaled` |
| Result | 0 passed, 1 failed, 0 ignored, 541 filtered; test exit 101, wrapper exit 1 |
| Bound | No timeout; 0.6495460839942098 seconds wrapper elapsed |
| First command | No numeric exit; SIGABRT (6), empty stdout/stderr, 46 ms, no truncation |
| Closure | Original group exited; journal records owned-group exit; cancellation owner reports exit and joined output |
| Files | Scratch empty; protected sentinel unchanged; diagnostics retained |
| Later cases | Bash exit 7 and owned sleep timeout were not reached |

The preserved root is `/private/tmp/kegari-nq02b-8944rt1y/.tmpOxKZ33`.
The exact request, binary, attempt marker, wrapper, source manifest, result and
logs are under `target/native-qualification/nq-02b-ac6cf23ac60b/`.
[Machine evidence](native-managed-command-recheck.json) records their hashes and
the source changes after this pin. The pin predates alpha.6's final host-launch
interlock; it cannot establish final-source native qualification even if passed.

An existing OS crash report matches both the journaled child PID and its parent:
`/Users/lgr/Library/Logs/DiagnosticReports/bash-2026-10-26-204451.ips`, SHA-256
`302ff607cdadd9b4025c11efff8ac2ba7ec62575f7ef3c673c8bd061ee299b3b`.
Its stack runs through `ignition_halt`, `boot_boot`, `ignite` and dyld's
`CacheFinder`. A small correlated summary is retained beside the test artifact;
unrelated crash-report metadata is not copied into repository evidence.
This locates the abort in loader bootstrap, before the script. It does not
identify the exact denied operation.

The installed `/System/Library/Sandbox/Profiles/dyld-support.sb` documents that
libignition opens `/` as an `openat` root. It grants literal root read/existence
access and nonrecursive read/existence access to fixed Cryptex ancestors. The
tested policy grants those ancestors only metadata reads. This is the strongest
concrete next correction, consistent with the crash location but not a proven
exact denial. Other narrowly documented dyld syscall requirements remain
candidates if that correction is insufficient.

Next add only the required literal-root/fixed-ancestor bootstrap grants, verify
that no recursive root, arbitrary executable mapping, write, network or Mach
authority is introduced, and prepare a fresh pinned native request. Do not import
the general system policy or rerun NQ-02B. The public managed-profile refusal
remains enforced. [NQ-02A's earlier failure](NATIVE-MANAGED-COMMAND.md) is retained.
