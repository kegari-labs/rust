# Review retry retains original round requirements

Status: all-target check, strict Clippy, all12 child continuation tests and all12
scheduler tests passed. Formatting,496-block inventory consistency and diff checks
also passed. This is24 unique focused passes, not a new full-suite or native run.

Independent review found that scheduling examines exact phase history, including
the retry-grant namespace. The first attempt under a new grant has no such
history, so a genuine retry of a previously marked review could reach generic
planning without its original opaque review-round capability.

The retry proof now privately captures exact marked review receipts across every
grant namespace for the same runtime, workspace, work and correction generation.
Its digest binds the capture and its current inventory check rereads it. Pure
planning, launch-request binding and the runtime's final planning path require
the actual round whenever that captured lineage needs one. Legacy unmarked
retries retain their existing path. Serialized markers cannot create a plan or
an admitted resumed-session capability.

The authored regression composes actual owned Git/source seals, Control round
preparation/model admission, a guarded in-process failed provider session and a
real DecisionManager grant against synthetic provider data. It checks the empty
new-grant history, rejected omission and genuine plan restoration. Existing
legacy retry fixtures check that unmarked retries remain valid. These fixtures
do not manufacture a native joined-completion registry or successful execution
qualification.

The reviewed first draft (manifest571d978a581edc93704c282c68bc15049b75c10e0760547757e089a9c7e3e575)
had a misplaced variable declaration identified during root static review before
installation. Corrected v2 is retained at
`/private/tmp/kegari-review-retry-round-guard-v2`, manifest
c9ff5444094c12ab1b62b6da03da7384eecd9800de8d0be76674ca2b13c7045e,
patch83405ee584343224b1ae744009ecc36dae9fc5d8f4bcea3c6e363b6a1a62ce11.
Five exact source paths plus root's two native omission-guard hunks are installed;
all original bytes are retained in
`/private/tmp/kegari-review-retry-round-guard-installed-source1`.
The broader paired recovery draft remains separate and uninstalled.

Source1 has615 inputs at
`sha256:223772a0e6b513f7ce0aba2ecf456772f98957b8c72cee50417d5f220b00fabb`.
The command runner retains actual command, logs, source snapshots and observed
Rust/Cargo1.99.1 identities before/after every invocation. It uses
`RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`
and locked Cargo. Session84681 runs all-target check, strict Clippy, the marked
first-grant regression and scheduler tests, with stop-on-failure sequencing.
No pass is inferred for an unexecuted suffix. The preceding766/0/2 full suite is
historical evidence for4244532's unchanged inputs, not a full-suite result for
this new guard source.

## Verified focused evidence

Session84681 completed check1 (15.220s), Clippy1 (15.862s), the new marked retry
regression (1/1;52.257s including compilation) and scheduler1 (12/12;0.106s).
Session97996 completed continuation1 (12/12;97.559s), fmt1 (1.295s), inventory1
(0.040s) and diff1 (0.014s). Both sessions are closed. The new marked test is
included in continuation1 and is not counted twice. All eight invocation logs,
source snapshots and before/after tool identities were verified; auxiliary
inventory inputs stayed unchanged. See the [machine evidence](review-retry-round-guard-checks.json).

No full suite was rerun for this isolated guard after the prior766/0/2 checkpoint;
validation targets the changed scheduling, binding and legacy/marked continuation
paths. Broader paired-recovery integration will require its own checks. Required
native probes, actual-agent/provider integration and human calibration remain
incomplete. This increment neither retries denied probes nor qualifies operation.
