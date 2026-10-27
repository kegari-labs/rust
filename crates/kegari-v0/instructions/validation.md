# Permitted validation and evidence preparation — version 4

Validate the exact isolated candidate/repository tree vector, declared test
resources and artifact identities supplied by the host. Do not validate a shared
staging environment containing unrelated parents as evidence for this result.
Discover relevant validation from the actual accessible repository and task
evidence, retaining configured and previously discovered obligations. Run only
role-permitted commands through actual host tools; an absent catalog is not a
pass or an execution grant. Record obligations and their revision; never drop
failures or select an easier subset. Source changes, including test repairs,
require the scoped child implementation path and repeated gates.

For child-local validation, use ordinary `source_read` and `context_read` to
inspect the actual task, instructions, configuration and scripts. Read
`context_read` with key `validation_evidence` to obtain the host's retained read
references and current plan. Submit `validation_propose` with `previous` (the
current plan digest, or null initially), a concrete `reason`, `supporting_reads`
(the observed reference digests), and `requirements`. Each requirement has a
stable `id`, a behavior `statement`, an evidence-based `rationale`, exact `argv`
and `cwd`, optional `services`, and optional `artifacts`. Do not invent references.
The first plan must explain why its checks cover the task; each revision must
explain what changed. Retain every prior obligation ID and all failure history.
Repository guidance can be stale or incomplete: assess the actual changed
behavior and missing tests instead of merely copying a documented command.

Run each enrolled obligation with `validation_run` using the returned
`plan_digest` and `obligation_id`. This tool checks the exact clean admitted
source and keeps actual execution evidence. Ordinary stdout-only commands need
no report wrapper: the host retains its own transcript. Optional artifact paths
must be under admitted scratch; no source output exclusions or permission
expansion are implied. Services require explicit bounded lifecycle/readiness
commands under the same authority. A new plan invalidates completion of an older
plan until its obligations have actually run. `command_run` may support discovery
and preparation, but its successful exit never satisfies a validation obligation.
If the current phase does not support discovered checks, report that actual
capability limitation; do not fabricate a configured check name or success.

Independent review receives the plan history and actual attempts and must assess
their adequacy. Mechanical check completion alone does not establish that the
suite covers the required behavior. Unknown execution, inaccessible prerequisites,
or lineage that cannot be recovered blocks the affected gate.

Record command argv, cwd, environment identity, operation/generation, tested
revision, start/end, exit/outcome and current-run artifact references. Missing,
stale, skipped, neutral, cancelled or unrun checks are not passed. Local success
does not establish hosted CI, review, staging or production success.

The parent summary must explain actual changed components, fulfilled acceptance
criteria, complete child outcomes, real local/hosted/review/integrated results,
remaining caveats and an evidence-based go/no-go assessment. Never fabricate
validation, examples or media. For visual work, recording must show the changed
end-to-end flow with safe data on the same isolated candidate. The host verifies
MP4 encoding, upload and parent association; a local recording or upload alone is
not publication. Nonvisual work needs a meaningful truthful summary and actual
results, not an artificial video. Human acceptance remains a separate explicit
operator decision on the evidence token.

When the host requests the parent result, submit `outcome_submit` with a `claim`
containing exactly these fields:

```json
{"schema_version":1,"executive_summary":"actual combined outcome and limits","touched_components":["observed component"],"fulfilled_criteria":["exact supplied criterion"],"caveats":[],"go":false}
```

List actual components and exact fulfilled criteria. Set `go` only when the
observed evidence supports it, and explain remaining caveats truthfully. Keep
actual artifact references in the tool's separate `artifact_refs` array. The
summary cannot establish any missing execution, hosted, review, visual-publication
or human-acceptance gate. Use the exact result schema in the supplied phase context
for another validation phase; do not invent extra evidence or gate-success fields.
