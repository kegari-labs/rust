# Permitted validation and evidence preparation — version 7

Validate the exact isolated candidate/repository tree vector, declared test
resources and artifact identities supplied by the host. Do not validate a shared
staging environment containing unrelated parents as evidence for this result.
Discover relevant validation from the actual accessible repository and task
evidence, retaining configured and previously discovered obligations. Run only
role-permitted commands through native Codex tools; an absent catalog is not a
pass or an execution grant. Record obligations and their revision; never drop
failures or select an easier subset. Source changes, including test repairs,
require the scoped child implementation path and repeated gates.

For child-local and assigned integrated validation, inspect actual task context
with `context_read` and the repository with native Codex tools. Discover commands
from accessible instructions, configuration, scripts and changed behavior. No
operator-maintained executable catalog or environment specification is required.
A missing prerequisite or unsupported capability is a blocker; never silently
substitute an unrelated tool or escape the supplied permissions.

Read `context_read` with key `validation_evidence` for the retained obligation
history and current plan. Submit `validation_propose` with `previous` (the current
plan digest, or null initially), a concrete `reason`, `supporting_reads` (actual
observed reference digests) and `requirements`. Each requirement has a stable
`id`, a behavior `statement`, an evidence-based `rationale`, exact `argv` and `cwd`,
and optional `services` and `artifacts`. These declarations describe intended
checks; Kegari does not execute their commands. Do not invent references. Explain
why the initial checks cover the task and what each revision changes. Retain
prior obligation IDs and all failure history. Repository guidance can be stale:
assess actual changed behavior instead of merely copying a documented command.

Run required checks using Codex's native execution tools. For background work,
use Codex's supported terminal/lifecycle facilities and observe readiness and
cleanup; do not assume arbitrary processes survive an interrupted app-server.
Read `context_read` with key `execution_results` for actual command/file item
observations retained from this Codex session. After a check completes, submit
`validation_record` with `plan_digest`, `obligation_id` and the observed `item_id`.
This associates the existing result with the obligation; it never launches a
command. The host checks session/turn/item/source attribution and the actual
result before accepting it. Missing or stale results cannot satisfy a check.
A successful unrelated command or a prose claim is not validation evidence.

A new plan invalidates completion of an older plan until its current obligations
have actual qualifying results. Optional artifacts must remain inside admitted
paths. Artifact declarations and result references do not expand permissions.
Report an unavailable validation capability precisely; never fabricate a check
name, item identity or result.

Independent review receives the plan history and actual attempts and must assess
their adequacy. Mechanical check completion alone does not establish that the
suite covers the required behavior. Unknown execution, inaccessible prerequisites,
or lineage that cannot be recovered blocks the affected gate.

For integrated validation, inspect the supplied complete candidate vector and
the E2E child's required outcome. The host derives integrated scope from that
assignment; a tool argument cannot select a different candidate or role. Preserve
all applicable configured and previously discovered obligations. Checks execute
from the E2E repository and may read only the other exact candidates/resources
admitted by the host. Source or test changes require scoped child work, followed
by fresh candidate construction and validation. A separate read-only Codex review
must assess the complete validation evidence before integrated readiness. The
later parent summary is coordination and cannot supply that independent review.

Record available command/cwd, operation/generation, thread/turn/item identity,
tested source, completion status, exit/outcome and current-run artifact references.
Do not manufacture timing, output, environment fingerprints or evidence that the
provider did not expose. Missing,
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
