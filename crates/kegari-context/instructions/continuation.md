# Correction, continuation and recovery — version 3

Use the host's current reconciliation findings, correction revisions and actual
progress. Earlier thread memory is historical context, not current gate truth.
Identify which latest correction watermark you processed and what changed in this
turn. Follow actionable steering within scope; if it races completion, the host
will retain it for a new admitted turn/session. Never assume a rejected steer was
received or that an identical patch preserves acceptance for changed behavior.

On continuation, work only on the remaining legitimate step. Preserve completed
results that the host still verifies, and report material new progress. Do not
repeat the entire initial prompt, reopen completed work because profile labels
changed, or reset attempt/review budgets. Report no_progress when nothing material
has advanced.

On recovery, inspect preserved files and the host-provided provider observations.
Do not reset/clean the directory, attach an unrelated thread, repeat a sent/unknown
comment/PR/merge/job, or claim remembered success establishes the current outcome.
Unknown effects require provider reconciliation before dependent actions. A
provider verification, safety/policy or manual-intervention blocker persists until
the supported required resolution is established; changing model/account/thread,
waiting or asking for retry cannot bypass it. Pause/stop and revoked permissions
remain effective. Report a precise blocker when safe continuation is impossible.

For authorized parent/child work, read `context_read` with key
`correction_state` before submitting the final outcome and after each correction
input. This returns the host's exact `watermark`, `input_digest` and
`transition_digest`. If its watermark is positive, add `processed_correction` to
the `outcome_submit` arguments alongside `claim` and `artifact_refs`, copying that
complete three-field object only after you have processed the current input and
completed the relevant work. Keep it outside the domain-specific `claim` object.
A null transition digest identifies the original assembled context. For example:

```json
{"claim":{"summary":"..."},"artifact_refs":[],"processed_correction":{"watermark":1,"input_digest":"sha256:...","transition_digest":null}}
```

Use the exact claim schema for your role; the abbreviated example does not replace
it. The host independently checks the current route, actual completed turn and
evidence. Acknowledging delivery does not consume a correction. A superseded or
unfinished turn cannot report a newer input as processed. An informational
classification without a route grants no new code change or inferred reversal.
Independent PR review has no Linear correction authority and omits this field.
